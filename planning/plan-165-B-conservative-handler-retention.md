# plan-165-B: A call is unable to fail only with positive proof

Last updated: 2026-10-04
Effort: small–medium (estimate: 45m–1.5h, excluding full-suite latency; was medium
1h–2h before the 2026-10-04 owner rule and bug-807/811/814 shrank the scope)
Depends on: plan-165-A (execution order only; no technical dependency — see A's ledger)
Status: Implemented 2026-10-05 (go-ahead: `/follow-plan 165`)

## Goal

Owner decision (2026-10-04): "With the trustworthy lists included in the
information pool, missing or unknown information counts as 'can fail', so the
handler is kept."

A call is treated as unable to fail only with **positive proof**:

- (a) a built-in whose registry `errors` is empty **and** that is covered by a
  raise-checking declared-errors guard — bug-698's native
  `src/codegen/builtins/tests/declared_errors_all.rs` (plus `fs_declared_errors.rs`),
  bug-698's `Body::Mfb` guard `mfb_body_declared_errors.rs` (TRAP-aware, with
  (package, member, code)-keyed exception tables), bug-807's
  `general_declared_errors.rs`, and bug-814's `Body::Rewrite` coverage in
  `mfb_body_declared_errors.rs`;
- (b) a user function proven by the fallibility fixpoint
  (`src/ir/fallible.rs:analyze`; its IR twin `src/ir/verify/trap_init.rs:total_functions`);
- (c) an imported package function with fallibility metadata in its `.mfp`.

Everything else counts as fallible and keeps its handler: a built-in not covered by
a guard, a name-keyed exemption, the `bits` package (no raise-checking guard today),
a package function without metadata, and a call site whose argument types are
unknown.

**Not "unknown": `&`.** String concatenation can raise `ErrOutOfMemory` but is the
owner's documented exception (2026-10-04, bug-814's open question; spec
`language/08_error-model.md` rule 11; `src/ir/fallible.rs:ARITHMETIC_OPERATORS`).
This plan never counts `&` fallible.

## Prerequisites

Met 2026-10-05: plan-165-A's boxes are all ticked (commits 38d7f2f8b, 954642f39);
`/follow-plan 165` is the go-ahead.

Plan-165-A complete before implementation (`rg -n '^Commit:|^- \[ \]'
planning/plan-165-A-handler-contracts.md`, or its archived path) unless the owner
reorders, and an explicit owner go-ahead to implement this plan (for example
`/follow-plan plan-165-B`). Reviewing or discussing the plan is not one.

## Current state and evidence

Measured on 2026-10-04 at main 16df1c451 with a scratch copy
(`git archive HEAD | tar -x -C /tmp/mfb165-src`) and a throw-away test that walks
`registry().packages()` and asks
`inline_builtin_is_infallible(callee, &[], &NoTypeKinds)` for every member
(`CARGO_TARGET_DIR=/tmp/mfb165-target cargo test --bin mfb probe165 -- --nocapture`).

**The census grants 39 members.** `src/codegen/builtins/mod.rs:inline_builtin_is_infallible`
grants by three routes: the registry branch (an `AbiInline` member whose
`native_member_declares_error` is `Some(false)` and that is not
`inline_builtin_raw_supported`), the name list `len | typeName`, and the name list
`contains | hasKey | getOr` over `native_builtin_target`; plus `toInt` over a single
enum argument (bug-679), which needs a real `TypeKinds` oracle
(`rg -n 'pub\(crate\) fn inline_builtin_is_infallible' -A70 src/codegen/builtins/mod.rs`).
Every one of the 39 has empty registry `errors`.

| Granted members | Raise-checking guard today |
|---|---|
| `strings.startsWith`, `endsWith`, `contains`, `byteLen`, `startsWithAny`, `endsWithAny`, `displayWidth`, `genCat` (8) | yes — `every_strings_member_declares_exactly_the_errors_its_lowering_raises` |
| `astrings.scalarLen` (1) | yes — `every_astrings_member_…` |
| `collections.contains`, `hasKey`, `getOr` (3) | yes — `every_collections_member_…` (no skip list) |
| `len`, `typeName` (2); `toInt(<enum>)` | yes — `general_declared_errors.rs` rows, all empty (`rg -n '\("(len|typeName)"|"Color"' src/codegen/builtins/tests/general_declared_errors.rs`) |
| `bits.band`, `bor`, `bxor`, `bnot`, `rl32`, `rr32`, `rl64`, `rr64`, `clz`, `ctz`, `popCount`, `bswap16`, `bswap32`, `bswap64` (14) | **no** — only the declaration check `shift_ops_are_fallible_the_rest_total` (`src/codegen/builtins/bits/mod.rs`), which reads `declares_error`, not the lowering |
| `astrings.fromString`, `writeSpans` (2) | **no** — `ASTRINGS_SKIP_MEMBERS` (record-layout fixture gap) |
| `canvas.geoFind`, `geoForget`, `sceneSnapshot` (3) | **no** — `CANVAS_SKIP_MEMBERS` |
| `json.sciParts`, `math.seed`, `money.setRounding`, `money.getRounding`, `regex.genCat`, `regex.scriptOf` (6) | **no** — no `every_<pkg>_member_…` test for `json`, `math`, `money`, `regex` |

Guard inventory: `rg -o 'fn every_(\w+?)_member_declares' -r '$1' src/codegen/builtins/tests/`
lists `app astrings audio big canvas collections crypto datetime io net os process
strings tcp term thread tls udp` (`declared_errors_all.rs`), `fs`, `general` and
`body_mfb`; skip lists: `rg -n 'SKIP_MEMBERS: ' src/codegen/builtins/tests/declared_errors_all.rs`.
So 14 grants are covered and 25 are not.

**Consumers of the census** (`rg -n 'inline_builtin_is_infallible\(' src --glob '!**/tests/**'`,
minus the definition and the `#[cfg(test)]` asserts in `builtins/mod.rs`): `builtins::inline_trap_unsupported` (codegen backstop),
`ir::fallible::Fallibility::call_is_fallible` (function fixpoint and inline-TRAP
lifting), `ir::verify::resources::check_inline_trap_scrutinee`
(`TYPE_INLINE_TRAP_DEAD_HANDLER`), `ir::verify::trap_init::Census::call_may_fail`
(bug-797 failure edges), `builder_control.rs:nir_call_is_infallible_builtin`
(plan-145-C effect-free / bug-816 `nir_value_cannot_fail`), and
`builder_values.rs:lower_value_inner` (dispatch to `lower_inline_infallible_raw`).
All six read the one predicate, so fixing it fixes all of them.

**Already fail-closed (measured; B pins, does not change):**

- Unknown argument types. After bug-807 (commit c97efac0d) every `toString` overload
  is fallible, so the former "unknown argument gets toString's infallible name
  verdict" hazard no longer exists. Every one of the 39 grants above is computed
  with `arg_types = &[]`; the only argument-dependent grant is `toInt(<enum>)`, and
  `NoTypeKinds` / an `Unknown` type answers `is_enum = false` (fallible); `replace`
  answers fallible for anything not provably a `List`
  (`rg -n 'fn arg_type_makes_inline_builtin_fallible' -A20 src/codegen/builtins/mod.rs`).
- Package functions. No `.mfp` carries fallibility metadata
  (`rg -n -i 'fallib|can_fail|declared_errors|infallible' src/binary_repr src/resolver/packages.rs src/ir/lower_link.rs`
  finds nothing). `Fallibility::call_is_fallible` answers fallible for any name not
  declared in the project, and `trap_init`'s module doc states "An imported or
  unknown callee is assumed to fail". Route (c) therefore grants nothing today.
- `mfb audit`. Bug-811 and bug-814 (commit a78859766) made audit read declared
  errors for every census built-in; `census_builtins_are_fallible_exactly_when_they_declare_an_error`
  (`src/audit/collect/source.rs`) pins audit to the declarations, so the earlier
  "audit is a separate, less precise proof source" concern is retired.

**Latent hazards in the grant routes (no current instance; the coverage test must
make them impossible):**

- `registry::native_member_declares_error` considers only `AbiInline`
  implementations (`rg -n 'pub\(crate\) fn native_member_declares_error' -A16 src/codegen/registry/mod.rs`):
  a member with a non-declaring inline overload and a declaring non-inline overload
  would be granted. Today every granted member is `AbiInline`-only or `Intrinsic`
  (probe output).
- The `contains | hasKey | getOr` name list matches **any** package's `AbiInline`
  member with that bare name, declared errors or not. Today all such members
  declare none (probe output).

Stale doc comments on `inline_builtin_is_infallible`,
`arg_type_makes_inline_builtin_fallible` and `Fallibility::call_is_fallible` still
call `toString` infallible; bug-825's "Also fix" owns them — do not duplicate.

## Design

1. **One proof rule, read from the registry.** A built-in is infallible iff every
   implementation of the member (not just its `AbiInline` ones) declares no error
   **and** the member is on the guarded set, plus the existing per-overload
   `toInt(<enum>)` grant. Delete the `len | typeName` and `contains | hasKey | getOr`
   name lists: `contains`/`hasKey`/`getOr` are already granted by the registry
   branch, and `len`/`typeName` (`Body::Intrinsic`, outside
   `native_member_declares_error`) get a registry-data path for `general`
   intrinsics. Restrict that path so it does not widen the set: `isNumeric`,
   `isEven`, `isOdd`, `isPositive`, `isNegative`, `isZero`, `isEmpty`, `isNotEmpty`
   also have empty `errors` but are not granted today (probe `probe165_general_empty`);
   granting them is a precision change outside this plan.
2. **Guard coverage for every grant, or the grant goes.** Add a raise-checking guard
   for `bits` (`every_bits_member_declares_exactly_the_errors_its_lowering_raises`
   through `declared_errors_sweep`, which already drives `AbiInline` lowerings for
   `strings`/`collections`). Do the same for the six `json`/`math`/`money`/`regex`
   grants (a member-filtered sweep is acceptable if a whole-package sweep surfaces
   unrelated mismatches — file those, do not fix them here). For the five skipped
   `astrings`/`canvas` members, close the record-layout fixture gap so the sweep
   lowers them; if that is not possible, they lose the grant and become fallible.
3. **Coverage-enforcing test.** A unit test enumerates every registry member (and
   the `toInt(<enum>)` overload) the census grants and fails unless each is covered
   by a raise-checking guard: lowered by a `declared_errors_all.rs`/`fs` sweep with
   the member not in a skip list, a `general_declared_errors.rs` row for every
   overload, or a `Body::Mfb`/`Body::Rewrite` entry in `mfb_body_declared_errors.rs`.
   The guarded set is data the test and the census share, so a new grant without a
   guard fails here first. User functions are covered by the fixpoint and need no
   entry.
4. **A member that loses its proof must still capture.** An unproved inline member
   with empty `errors` is neither raw-supported nor infallible today, which trips
   the `inline_trap_unsupported` codegen backstop. Route every unproved `AbiInline`
   member through real raw capture (`builder_values.rs:lower_inline_builtin_raw`)
   by making `inline_builtin_raw_supported` answer `true` for it. Retaining a
   handler in IR is insufficient if native lowering still wraps the call as `Ok`.
5. **Fail-closed pins.** Tests that an unknown argument type cannot grant
   (`toInt(Unknown)`, `replace(Unknown, …)`), and that a function imported from a
   built `.mfp` package keeps a live handler and is a bug-797 failure edge.
   Route (c) — `.mfp` fallibility metadata — does not exist and is not added here.
6. **`&` stays outside.** No test or census change counts `&` fallible.

## Non-goals and compatibility

Do not widen the infallible set, change any declared error list to manufacture a
proof, add a runtime fallback, change the `.mfp` format or the ABI, or count `&` as
fallible. Unproved handlers may make binaries larger, as the owner prefers.
If a member loses its grant (only possible for the five skipped `astrings`/`canvas`
members, or a guard that finds a real raise), source compatibility changes on
purpose: a function handler that reads a local declared after such a call becomes
`TYPE_TRAP_LOCAL_UNINITIALIZED`, and its dead-handler warning disappears. That is
the correct verdict; fix affected fixtures by moving the declaration before the
call, as bug-797 did for bug-465's tcp/tls EOF fixtures, and list each one in the
commit. A guard that finds a real undeclared raise is a bug: file it with its repro.
Byte identity is not the correctness criterion for intentionally retained checks.

## Phases

### Phase 1 — Coverage test and fail-closed pins (RED)

- [x] Re-measure the grant list and consumers (`rg -n 'inline_builtin_is_infallible\(' src`).
      Grants: the coverage test below listed exactly the 25 uncovered grants of the
      table at `f23af579c`. Consumers unchanged: fallible.rs, verify/resources.rs,
      verify/trap_init.rs, builder_values.rs, builder_control.rs (+ test asserts).
- [x] Add the coverage-enforcing test (Design 3); it must fail at HEAD naming the 25
      uncovered grants (`bits` ×14, `astrings` ×2, `canvas` ×3, `json`/`math`/
      `money`/`regex` ×6) and pass the 14 covered ones.
      `infallible_proof_coverage::every_inline_builtin_census_grant_is_raise_checked`
      failed with "25 census grant(s) have no raise-checking guard" listing exactly
      those members (`cargo test --bin mfb -- inline_builtin`, before Phase 2).
- [x] Add a test that a member with a non-declaring inline implementation and a
      declaring non-inline one is not granted, and that the `contains`-style name
      route cannot grant a declaring member (a registry-built fixture, not a real
      member). `an_inline_builtin_with_a_declaring_non_inline_overload_is_not_granted`,
      `an_inline_builtin_named_like_a_pure_query_is_not_granted_by_name` (fixtures
      over `totality_proof::member_is_proven_total`).
- [x] Add the fail-closed pins (Design 5); these pass at HEAD and stay passing.
      `an_inline_builtin_with_unknown_argument_types_is_not_granted`,
      `an_inline_builtin_census_never_grants_an_unknown_name`, and
      `tests/runtime/rt_imported_function_fallibility.rs` (a built `.mfp` import keeps
      a live handler and is a failure edge; `2 passed`).
- [x] Add a verifier test (alongside bug-797's in `src/ir/verify/tests.rs`) that an
      unproved built-in call is a failure edge for handler-local readability.
      `an_unproved_built_in_call_is_a_failure_edge` (`isEmpty`) and the control
      `a_proven_total_built_in_call_is_not_a_failure_edge` (`len`, `bits.band`).

Acceptance: `cargo test --bin mfb inline_builtin` runs the new tests; only the
coverage test fails, listing exactly the uncovered grants (estimate: minutes).
Met for the coverage test (25 listed). The two registry-fixture tests need the
factored predicate Phase 2 introduces, so they were written with it and are not
separately RED (see Corrections).
Commit: 695295c48 (shared with Phase 2)

### Phase 2 — Guards, one proof rule, capture, spec

- [x] Add the `bits` guard and the `json`/`math`/`money`/`regex` coverage; resolve
      or drop the five skipped members (Design 2). Five whole-package sweeps in
      `declared_errors_all.rs`; the skip lists are gone entirely —
      `declared_errors_sweep::PROGRAM_FIXTURES` lowers `astrings`, `canvas` and
      `crypto` against a real program's type model and globals, so all 15 canvas
      members, both astrings members and `crypto::generate` are now swept. The
      sweeps found real undeclared raises, fixed as bug-828 (`json::sciParts`,
      `astrings::fromString`/`writeSpans`, every `math` list form: ErrOutOfMemory).
- [x] Replace the name lists with the registry rule (Design 1) and make unproved
      inline members raw-capable (Design 4); add a runtime fixture for any member
      that lost its grant, proving the handler is live and captures.
      `totality_proof::{member_is_proven_total, inline_member_is_proven_total}`;
      `len | typeName` → `RAISE_CHECKED_GENERAL_TOTAL`; `contains | hasKey | getOr`
      deleted. Lost grants: `json::sciParts` (internal-only, not user-callable),
      `astrings::fromString`, `astrings::writeSpans` (internal-only) — fixture
      `tests/rt-behavior/trap/inline-trap-unproved-builtin-rt` (no dead-handler
      warning; builds through the raw capture; prints the value).
- [x] Update spec `language/08_error-model.md` rule 11's dead-handler sentence to
      state the positive-proof rule and its guard ownership (coordinate wording with
      bug-825's correction of the same rule; do not reintroduce the `&` sentence it
      fixes); document proof ownership in code. Rule 11's "infallible only if"
      sentence replaced; `&` text untouched; `totality_proof.rs` module docs.
- [x] Add valid/invalid fixtures and run the builtin oracle plus runtime regressions.
      Valid `inline-trap-unproved-builtin-rt`, invalid
      `tests/syntax/trap/trap-local-after-unproved-builtin-invalid`
      (`TYPE_TRAP_LOCAL_UNINITIALIZED` after `fromString`, none after `len`):
      `scripts/test-accept.sh … ` on both plus `inline-trap-infallible-builtin-valid`
      → `acceptance tests passed (3 test(s) ran)`. "Builtin oracle" = the
      declared-errors guards (`cargo test --bin mfb -- declared_errors mfb_body` green);
      runtime regressions: see Phase 2 acceptance.

Acceptance: `cargo test --bin mfb inline_builtin`, `cargo test --bin mfb declared_errors`,
`cargo test --bin mfb census_builtins_are_fallible_exactly_when_they_declare_an_error`
and the new fixtures pass; the coverage test is green with no allowlist
(estimate: minutes after build). Met: `cargo test --bin mfb -- spec declared_errors
mfb_body inline_builtin failure_edge builtins::tests census math:: man` →
`451 passed; 0 failed` (includes the coverage test with an empty
`RAISE_CHECK_SKIPPED_MEMBERS`, and the audit census test); runtime regressions
`cargo test --test rt_inline_trap_nested_call --test rt_inline_trap_raising_operator
--test rt_inline_trap_success_alloc --test rt_imported_function_fallibility --test
rt_error_model_recovery` → all `ok` (21+20+2+2+4).
Commit: 695295c48

## Open decisions

None about conservatism: the owner's rule decides it. Whether to newly grant the
eight empty-`errors` `general` predicates is a separate precision decision, not
part of this plan.

## Validation discipline

New runtime tests execute compiled programs and assert stdout, error codes and
side-effect order. Add matching valid/invalid fixtures under the repository's
`tests/rt-behavior/trap` and `tests/syntax/trap` trees. Do not weaken existing
assertions: record history, protected behavior, consumers and an independent
contract/reproduction proving each changed expectation wrong.

Run formatting and scoped tests during implementation. At the end of this
sub-plan, run `cargo test --no-fail-fast` and
`scripts/test-accept.sh target/debug/mfb target/accept-error-model` before any
existing golden correction; full cargo testing includes the artifact gate.
These broad checks are required by the repository and may take hours (estimate).
After justified corrections, rerun affected checks and required final gates.
Verify specs with `cargo build`, `cargo test --bin mfb spec`, and rendered topics.

## Corrections

Planning only. An initial implementation attempt was removed at the owner's
instruction; no compiler or test changes are retained. Any experimental success
is not acceptance evidence for a future implementation.

2026-10-03 review: bug-797 landed after this plan was written and made
`src/ir/verify/trap_init.rs` a consumer of `inline_builtin_is_infallible`. Added it
to the audit, the design and a verifier test, and recorded the intended
`TYPE_TRAP_LOCAL_UNINITIALIZED` source-compatibility change. Replaced the
session-specific authorization text with a reusable go-ahead condition.

2026-10-04 rewrite: adopted the owner's positive-proof rule. Removed premises that
are no longer true: the `toString` unknown-argument case (bug-807 made every
`toString` fallible), the stale `sed -n '285,320p'` citation, and the `mfb audit`
concern (bug-811/814; pinned by
`census_builtins_are_fallible_exactly_when_they_declare_an_error`). Replaced the
"audit each proof by hand" phase with a measured 39-grant inventory, guard coverage
for the 25 uncovered grants, and a coverage-enforcing test. Effort re-estimated from
1h–2h to 45m–1.5h.

2026-10-05 (implementation):
- Phase 1's acceptance ("only the coverage test fails") was miscalibrated for the
  two registry-fixture tests: a fixture cannot be fed to the global-registry census,
  so they test the factored `totality_proof::member_is_proven_total` that Phase 2
  introduces and were written with it. Phases 1 and 2 landed in one commit.
- Design 2 said to file, not fix, mismatches a whole-package sweep surfaces. The
  repository rule ("never leave a bug you found") outranks that: the `math` list
  forms' and `json::sciParts`' undeclared ErrOutOfMemory are fixed as bug-828, as
  are `astrings::fromString`/`writeSpans` (which lost their grant, as Design 2
  anticipated). `math::log`/`log10`'s unreachable Fixed ErrOverflow is recorded as
  an `unreachable_own_raise` with its proof.
- The five skipped members were resolved, not dropped, and so were the other ten
  canvas members and `crypto::generate`: `declared_errors_sweep::PROGRAM_FIXTURES`
  lowers a package against a real program's type model and globals. The skip
  machinery stays (empty) as the place a future gap must be recorded.
- Adding OOM to `math`'s list forms charged `color`/`vector` `Body::Mfb` bodies the
  list forms' errors for scalar calls; `mfb_body_declared_errors` now narrows a
  provably scalar argument to the scalar overloads (sound: no infix operator yields
  a collection).
- Design 4's raw path for an unproved, non-declaring inline member is live code
  but no member takes it today: every such member is now either guarded (granted)
  or declares an error (`infallible_proof_coverage` enumerates them).
- "Builtin oracle" in the Phase 2 task read as the declared-errors guards; no other
  builtin oracle exists for these members (`tools/oracles` covers crypto/compress/
  aarch64 decode only).
