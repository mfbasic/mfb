# plan-26-C: expectTrap/expectNTrap parity + diagnostics cleanup

Last updated: 2026-07-06
Effort: medium (1h–2h)

Bring the test-framework trap assertions (`expectTrap`/`expectNTrap`) to the same
uniform "a built-in is just a call" model that plan-26-A/B established for inline
`TRAP`, and retire the now-dead front-end gates and their diagnostics-table rows.
After this sub-plan, the developer sees one consistent story: `TRAP`, `expectTrap`,
and `expectNTrap` all accept any callable; provably-infallible ones are allowed
(the assertion behaves exactly as it would for an infallible user FUNC), and the
callback members are fully supported.

The single behavioral outcome: `expectTrap(collections::transform(list, mayFail))`
and `expectNTrap(len(list))` both compile with no diagnostic and evaluate at runtime
against the real trap outcome.

Depends on: plan-26-A and plan-26-B (the inline-TRAP codegen the assertions reuse).

It complements:

- `./mfb spec language tooling-and-auditability` and the test-framework docs
  (specs under `src/docs/spec/**`).
- `./mfb spec diagnostics error-codes`
  (`src/docs/spec/diagnostics/02_error-codes.md` — the rows this plan retires/narrows).

## 1. Goal

- `expectTrap`/`expectNTrap` accept any call the inline-TRAP machinery now supports,
  including infallible built-ins and the callback members.
- `TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE` (`2-208-0006`) is **kept, narrowed** to the
  non-call / package-constant case; `TESTING_EXPECT_TRAP_INLINE_BUILTIN`
  (`2-208-0007`) is **fully retired** — so no case is rejected for `expectTrap` that
  inline `TRAP` accepts, and vice-versa.
- The diagnostics table (`src/rules/table.rs`) and `mfb spec diagnostics error-codes`
  reflect the final rule set; no dangling rule names referenced from retired code.

### Non-goals (explicit constraints)

- No change to the assertion runtime semantics beyond what acceptance requires:
  `expectTrap` still passes iff the guarded call traps; `expectNTrap` still passes
  iff it does not. Retiring the *compile-time* gate does not change *runtime* pass/fail.
- No language-surface change beyond removing rejections.
- No change to error-code numbering for rules that survive; retired codes are removed,
  not renumbered.

## 2. Current State

- Test-framework gate: `check_trap_guardable`
  (`src/syntaxcheck/inference.rs:1072-1112`) drives `expectTrap`/`expectNTrap`
  (`EXPECT_TRAP`/`EXPECT_NTRAP`, `inference.rs:1040`/`:1060`). It rejects:
  - non-call scrutinee → `TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE` (`:1084`);
  - package constant or infallible inline built-in → `..._REQUIRES_FALLIBLE`
    (`:1096`);
  - fallible-but-unsupported inline built-in → `TESTING_EXPECT_TRAP_INLINE_BUILTIN`
    (`:1102`).
- Rule rows: `src/rules/table.rs:1191` (`2-208-0006`) and `:1197` (`2-208-0007`); the
  inline-TRAP siblings at `:676`/`:682` (`2-203-0069`/`2-203-0102`) are narrowed/
  retired by 26-A/26-B.
- After 26-A/B, `inline_builtin_is_infallible` no longer implies "reject", and
  `inline_trap_unsupported` returns false for the callback members — so the two
  branches at `check_trap_guardable:1093-1102` are the last place still rejecting
  cases that inline `TRAP` now accepts.

## 3. Design Overview

`check_trap_guardable` should reject exactly what inline `TRAP` rejects — nothing
more. Post-26-A/B that is: a **non-call scrutinee** and a **package constant** (no
runtime call to trap). Everything else (infallible built-ins, callback members,
index members, user FUNCs) is accepted. This makes the assertion gate a thin alias
of the inline-TRAP fallibility rule rather than an independent, stricter policy.

Then retire the two `2-208` rows and any inline-TRAP rows made fully dead, updating
the table and the embedded diagnostics spec together (the `error-codes` table is the
build input for `errorCode::`, so it must stay consistent).

## 4. Detailed Design

### 4.1 Narrow `check_trap_guardable` (`src/syntaxcheck/inference.rs:1072`)

- Keep the non-call rejection (`:1084`) — a non-call has nothing to trap.
- Keep the **package-constant** rejection but drop the
  `inline_builtin_is_infallible` half of the `:1093` condition: an infallible
  built-in is now accepted (parity with inline `TRAP` from 26-A).
- Delete the `inline_trap_unsupported` branch (`:1102-1112`): the callback members
  are raw-supported after 26-B, and no other inline target should reach here as
  unsupported. If an unsupported target can still exist, keep a single narrowed
  diagnostic shared with inline TRAP; otherwise remove it.
- Net: `expectTrap`/`expectNTrap` reject only non-calls and package constants.

### 4.2 Retire diagnostics rows (`src/rules/table.rs` + spec)

- Keep `2-208-0006` (`TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE`), narrowing its message
  to "non-call / package-constant scrutinee" (decided — preserves a located
  diagnostic for `expectTrap(42)`).
- Remove `2-208-0007` (`TESTING_EXPECT_TRAP_INLINE_BUILTIN`) — fully dead after 26-B.
- Coordinate any inline-TRAP row retirement deferred from 26-B
  (`TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`, `2-203-0102`): remove it here if it is now
  unreferenced.
- Update `src/docs/spec/diagnostics/02_error-codes.md` in the **same commit** so the
  `error-codes` table (the `errorCode::` build input) matches `table.rs`. Grep the
  tree for each retired rule name to prove no code still references it.

### 4.3 Test-framework docs

- Update the test-framework spec topic and any `mfb man` / test docs to state that
  `expectTrap`/`expectNTrap` accept any call; on an infallible callee `expectTrap`
  will simply always fail at runtime (no trap) and `expectNTrap` always pass — the
  same as for an infallible user FUNC.

## Layout / ABI Impact

None. Diagnostics-table row removal changes the `errorCode::` build input; ensure the
`errorCode::` enum/codes for surviving rules are unchanged (only retired codes drop).

## Phases

### Phase 1 — narrow the assertion gate

- [x] `src/syntaxcheck/inference.rs:1072-1112`: drop the infallible-built-in and
      `inline_trap_unsupported` rejections; keep non-call and package-constant.
- [x] Update the assertion-gate unit tests (`inference.rs` test module) to assert the
      new acceptance set (infallible + callback members pass; non-call + constant fail).

Acceptance: `expectTrap(collections::transform(list, mayFail))`,
`expectNTrap(len(list))`, and `expectTrap(collections::get(list, i))` all pass syntax
check; `expectTrap(42)` and `expectTrap(<package-constant>)` still reject.
Commit: —

### Phase 2 — retire/narrow diagnostics rows (table + spec together)

- [x] `src/rules/table.rs`: retire `2-208-0007`; keep `2-208-0006` with the narrowed
      message; retire `2-203-0102` if now unreferenced (grep to confirm).
- [x] `src/docs/spec/diagnostics/02_error-codes.md`: mirror the exact same changes.
- [x] `grep -rn` each retired rule name across `src/` and `tests/` to prove zero
      dangling references (code, fixtures, goldens).

Acceptance: `cargo build` clean; `mfb spec diagnostics error-codes` matches
`table.rs`; no retired rule name appears anywhere in `src/` or `tests/`.
Commit: —

### Phase 3 — fixtures + docs

- [x] Test-framework fixtures: a TESTING block using `expectTrap` over a callback
      member and `expectNTrap` over an infallible built-in, with a runtime proof the
      assertions evaluate correctly (`mfb test` passes/fails as expected).
- [x] Remove or convert stale invalid fixtures that asserted the retired
      `TESTING_EXPECT_TRAP_*` rejections.
- [x] Update the test-framework spec topic and relevant `mfb man` pages (§4.3).

Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green;
`mfb test` on the new fixture behaves correctly.
Commit: —

## Validation Plan

- Function/framework tests: TESTING-block fixtures exercising `expectTrap`/
  `expectNTrap` over infallible, index, and callback callees, valid and invalid.
- Runtime proof: `mfb test` on a fixture where `expectTrap(mayFail(...))` passes and
  `expectTrap(len(x))` fails (no trap) — proving the gate no longer masks runtime
  semantics.
- Doc sync: `mfb spec diagnostics error-codes`, test-framework spec topic, affected
  `mfb man` pages.
- Acceptance: `scripts/test-accept.sh`.

## Open Decisions

- **Retire `2-208-0006` entirely vs. keep one narrowed rule. — DECIDED: keep,
  narrowed.** Keep `TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE` (`2-208-0006`) with a
  narrowed message "trap assertion requires a call (got a non-call / package
  constant)". It stays a distinct located diagnostic for the genuine mistake
  (`expectTrap(42)`), parallel to inline `TRAP`'s non-call message. Only
  `2-208-0007` (`TESTING_EXPECT_TRAP_INLINE_BUILTIN`) is fully retired.
- **`TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` ownership.** Retire here (this sub-plan owns
  the diagnostics-table cleanup) unless 26-B already removed it.

## Non-Goals

- Any change to inline-TRAP codegen (done in 26-A/B).
- Changing `expectTrap`/`expectNTrap` runtime pass/fail semantics.

## Summary

Pure convergence: make the assertion gate reject exactly what inline `TRAP` rejects,
then delete the diagnostics rows the convergence makes dead — keeping `table.rs` and
the embedded `error-codes` spec in lockstep so `errorCode::` stays consistent.
