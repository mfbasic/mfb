# bug-657: a `UNION` with a scalar member passes the verifier and dies in codegen

Last updated: 2026-09-19
Effort: small → **re-estimated: medium–large.** Three approaches were tried and all
three fail; see "Attempts that do not work" below. The remaining route is an IR
format change.
Severity: MEDIUM
Class: Correctness (missing diagnostic)

Status: Open — root cause confirmed, three fixes ruled out by measurement
Regression Test: none yet — see Phase 1

## Attempts that do not work (2026-09-19)

All three were built and run, not reasoned about. Recording them because each looks
correct until it is measured, and the next person will reach for them in this order.

**1. Re-parse the member's spelling — ARCHITECTURALLY FORBIDDEN.**

```rust
let builtin = !matches!(ParameterType::parse(&variant.name), ParameterType::Named(_));
```

This is the obvious one-liner and it works. It is also exactly what plan-111's ratchet
gate bans: `tests/guards/no_type_strings.rs` holds `parse_sites / ir` at **0**, a
documented hard floor, and "a spelling flowing into a decision" is the class it exists
to stop. The gate fails the build with *"a type spelling reached a decision below the
AST"*. Budgets are asserted tight in BOTH directions, so raising it is not an option
either — that is the "silent allowance" the gate's own docs call out.

**2. Allowlist over `self.records` — SILENTLY INERT.**

`records` looks like "the declared record types". It is not: the `"union"` arm of
`TypeEnv::build` (`src/ir/verify/mod.rs`, *"Each variant is a record type in its own
right; register its payload fields so `variant.field` accesses resolve"*) registers
EVERY variant name as a record entry. For the reproduction below, dumping the table's
keys gives exactly:

    records_keys=["String", "Integer"]

— the two bogus members of the union under test. So the check stops firing altogether
and BOTH union fixtures build clean. Any allowlist over `records` is dead on arrival.

**3. Allowlist over `self.type_decl_info` — 12 FALSE POSITIVES.**

`type_decl_info` IS unpolluted (it is built from `project.types` alone), and this
version passes the ratchet, rejects the reproduction correctly, keeps
`types-union-member-invalid` reporting exactly as before, and passes the **whole
1,486-test acceptance suite**. It still fails: 12 `ir::verify` unit tests go red with
*"expected clean, got [TYPE_UNION_MEMBER_REQUIRES_TYPE, …]"* —

    accepts_union_variant_return, accepts_union_with_included_union,
    accepts_union_wrap_of_real_variant, accepts_exhaustive_union_match,
    accepts_union_match_with_else, accepts_mut_list_of_union_defaultable_empty,
    accepts_mut_map_value_union_defaultable_empty,
    accepts_mut_record_with_list_of_union_field,
    accepts_function_with_union_and_result_value_shapes,
    func_returns_via_exhaustive_union_match, func_returns_via_match_else,
    match_guard_reads_union_extract_bind

because they build minimal IR in which the variant records are never declared —
`project(vec![f], vec![u])`, with only the union in `project.types`. Those tests assert
the verifier ACCEPTS such a union, so the four-question rule applies and the tests win:
nothing here proves them wrong, and a real program's IR is not the only IR the verifier
must accept.

**What that leaves.** The decision needs the member's *elaborated type*, which already
exists one layer up: `HirUnionVariant` carries `type_: ParameterType`, parsed at the
sanctioned boundary (`src/hir/mod.rs`, `ParameterType::parse(&variant.name)`).
`lower_variant` (`src/ir/lower.rs:592`) then throws it away, keeping only
`name: String`. Carry it into `IrVariant` and the verifier can ask
`matches!(variant.type_, ParameterType::Named(_))` — a pure type-domain decision, no
spelling, no table lookup, and correct for hand-built IR too. That is an IR **format**
change: `IrVariant` plus `ir/json.rs`, `ir/binary.rs` and every constructor
(`variant_corpus_tests.rs`, `verify/tests.rs`, `binary_repr/tests/writer_tests.rs`),
with the `.ir` goldens and the binary round-trip gates behind it.

`UNION Shape` with `Integer` and `String` as members is accepted by the semantic verifier
and then fails in code generation with an internal error:

```
error: native code union wrap member 'Integer' is not a record while lowering bind sh AS Shape
```

The rule that should have caught it already exists — `TYPE_UNION_MEMBER_REQUIRES_TYPE`
(`2-203-0064`), whose message is *"union members must name concrete TYPE declarations"* —
but its enforcement only rejects a member that is a declared **union or enum**, never a
built-in scalar. The spec agrees with the rule, not with the code: §4.3's union members
are all record `TYPE`s, and `MATCH` reaches a payload that "members need not share a
field", which presumes a record.

**The single correct behavior a fix produces:** the program below is rejected at the
`UNION` declaration with a located `TYPE_UNION_MEMBER_REQUIRES_TYPE` naming the offending
member, and no program that builds today changes.

References:

- `src/ir/verify/types.rs:107-125` — the enforcement loop. Its own comment says "Each
  named member must be a concrete TYPE (record)", then tests only
  `self.unions.contains_key` / `self.enums.contains_key`; a name that is neither — a
  built-in scalar — falls through as acceptable.
- `src/rules/table.rs:705` — the rule and its message.
- `src/docs/spec/language/04_types.md` §4.3 — every documented union member is a record
  `TYPE`.
- Found while enumerating thread message types for bug-650 Phase 1 (the sweep needed a
  data union and reached for the scalar spelling first).

## Failing Reproduction

```
IMPORT io

UNION Shape
  Integer
  String
END UNION

SUB main()
  LET sh AS Shape = 5
  io::print("ok")
END SUB
```

`mfb build --debug` at `3d49a969e`, macOS aarch64:

- Observed: `error: native code union wrap member 'Integer' is not a record while lowering
  bind sh AS Shape`, exit non-zero. No file, no line, no rule code.
- Expected: `TYPE_UNION_MEMBER_REQUIRES_TYPE` at the `UNION` declaration, naming `Integer`.

Verified on a clean main-tip compiler. The record-member spelling of the same union
(`UNION Shape` over two `TYPE`s) builds and sends across a thread boundary fine, so this is
specific to the scalar member.

## Root Cause

Localized: the verifier's member check is written as a blocklist of the two *declared*
kinds it knew about (union, enum) rather than as a test of "is a concrete TYPE".
Anything else — every built-in scalar, `String`, a collection spelling — passes. Codegen
then reaches `union wrap member … is not a record` and has no diagnostic vocabulary
left.

The *fix* is harder than the root cause, because the verifier has no sound way to ask
the question at that point: the spelling is off limits (attempt 1), `records` is
polluted with the variants themselves (attempt 2), and the declaration table is not
populated for the hand-built IR the verifier's own tests use (attempt 3).

## Goal

- The reproduction is rejected with the existing located rule.
- Every union that builds today still builds: the check must be "member is a record",
  computed the same way codegen's `is not a record` test is, so the two cannot drift.

### Non-goals (must NOT change)

- `INCLUDES` member merging, resource-union variants, or `TYPE_UNION_INCLUDE_REQUIRES_UNION`.

## Blast Radius

- Every `UNION` declaration in `tests/`, `examples/` and the built-in packages must still
  pass. No fixture uses a scalar member today (`grep` over `tests/**/main.mfb` for a union
  body line naming a scalar → 0 hits), so the new rejection should be inert on the tree —
  confirm by running the full suite, not by reading.
- Diagnostic goldens: a new rejection adds a `build.log` line wherever a fixture trips it.
  Expected to be none, per the grep above.

## Phases

### Phase 1 — failing test + audit

- [x] Syntax fixture asserting the located diagnostic; confirmed RED. **Written and then
      removed again** along with the reverted fix — a committed fixture whose golden
      shows a diagnostic the compiler does not emit would be a false record. Recreate it
      from the reproduction above; it is four lines.
- [x] Confirm no existing union in the tree is newly rejected — acceptance 1,486 tests
      passed under attempt 3, and `examples/`, `tools/` and `src/docs/` carry no union
      with a scalar member either. That part of the audit stands whatever the fix is.

Commit: — (nothing landed)

### Phase 2 — the fix

- [ ] Carry `HirUnionVariant::type_` into `IrVariant` and decide on the type. The three
      cheaper routes are ruled out above — do not re-try them.

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

The diagnostic for this exact mistake already exists; its enforcement tests the wrong
predicate, so the error surfaces as an internal codegen failure instead. Fixing it
needs the variant's elaborated type carried into the IR — the verifier cannot answer
the question soundly from what it currently holds.
